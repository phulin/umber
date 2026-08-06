use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail, ensure};
use sha2::{Digest, Sha256};

use crate::cohort_transaction::CohortCase;

const TRANSACTION_PREFIX: &str = ".fixture-layout-transaction-";
const OWNER_MARKER: &str = "owner";
const COMMITTED_MARKER: &str = "committed";
const TRANSACTION_SCHEMA: &str = "umber-fixture-layout-transaction-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Plan,
    Apply,
}

pub fn seal_classic_case(root: &Path, case: &str) -> Result<()> {
    validate_component(case)?;
    let mut inventory = read_regular_inventory_recursive(root)?;
    inventory.remove("case.inventory");
    inventory.remove("case.json");
    let metadata = classic_case_metadata(case, &inventory)?;
    inventory.insert("case.json".to_owned(), metadata.clone());
    fs::write(root.join("case.json"), metadata)?;
    test_support::closed_case::seal_candidate_inventory(
        root,
        inventory.keys().map(String::as_str),
    )?;
    Ok(())
}

/// Publishes one already-closed case through the same plan and transaction
/// used by layout, PDF, and externally staged cohort publication.
pub(crate) fn publish_case_inventory(
    authority_root: &Path,
    destination: &Path,
    inventory: BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    ensure!(
        destination.starts_with(authority_root),
        "fixture destination is outside its authority root"
    );
    let area = destination
        .parent()
        .context("fixture destination has no parent")?
        .to_owned();
    let case = file_name(destination)?;
    let display_area = area
        .strip_prefix(authority_root)
        .unwrap_or(&area)
        .to_string_lossy()
        .into_owned();
    let authorities = destination
        .exists()
        .then(|| vec![destination.to_owned()])
        .unwrap_or_default();
    let plan = CasePlan {
        cases: vec![ArtifactSpec {
            area,
            case,
            display_area,
            layout: Layout::Flat,
            inventory,
            authorities,
            ownership_staged: None,
            ownership_authorities: destination
                .exists()
                .then(|| vec![destination.to_owned()])
                .unwrap_or_default(),
        }],
    };
    let digest = plan.transaction_digest();
    ensure!(
        retained_transactions(authority_root, &digest)?
            .committed
            .is_empty(),
        "owned committed transaction exists but installed fixture is incomplete"
    );
    AtomicCaseTransaction::new(authority_root, plan, digest, &RealFs)?.apply()
}

pub(crate) fn publish_file_in_tree(
    authority_root: &Path,
    tree: &Path,
    relative: &Path,
    bytes: Vec<u8>,
) -> Result<bool> {
    validate_relative(&relative.to_string_lossy())?;
    let mut inventory = read_regular_inventory_recursive(tree)?;
    let name = relative.to_string_lossy().into_owned();
    if inventory.get(&name) == Some(&bytes) {
        return Ok(false);
    }
    inventory.insert(name, bytes);
    publish_case_inventory(authority_root, tree, inventory)?;
    Ok(true)
}

pub(crate) fn run_staged_cohort(
    repository: &Path,
    cases: &[CohortCase],
    mode: Mode,
) -> Result<String> {
    run_staged_cohort_with_fs(repository, cases, mode, &RealFs)
}

pub(crate) fn run_staged_cohort_with_fs(
    repository: &Path,
    cases: &[CohortCase],
    mode: Mode,
    io: &dyn TransactionFs,
) -> Result<String> {
    validate_cohort_path_ownership(cases)?;
    let mut planned = Vec::new();
    ensure!(!cases.is_empty(), "cohort plan has no cases");
    for case in cases {
        let staged = repository.join(&case.staged);
        let inventory = crate::cohort_transaction::validate_staged_case(&staged)?;
        let destination = repository.join(&case.destination);
        let destination_parent = destination
            .parent()
            .context("cohort destination has no parent")?;
        let parent_metadata = fs::symlink_metadata(destination_parent).with_context(|| {
            format!(
                "inspect cohort destination parent {}",
                destination_parent.display()
            )
        })?;
        ensure!(
            parent_metadata.is_dir() && !parent_metadata.file_type().is_symlink(),
            "cohort destination parent is not a real directory"
        );
        let complete =
            destination.is_dir() && read_regular_inventory_recursive(&destination)? == inventory;
        let mut case_authorities = Vec::new();
        if !complete {
            for authority in &case.authorities {
                crate::cohort_transaction::validate_git_authority(repository, authority)?;
                case_authorities.push(repository.join(authority));
            }
            ensure!(
                !destination.exists() || case_authorities.contains(&destination),
                "existing destination {} must be a named authority",
                case.destination
            );
        }
        let parent = destination_parent.to_owned();
        let name = file_name(&destination)?;
        planned.push(ArtifactSpec {
            area: parent,
            case: name,
            display_area: Path::new(&case.destination)
                .parent()
                .context("cohort destination has no parent")?
                .to_string_lossy()
                .into_owned(),
            layout: if complete {
                Layout::Directory
            } else {
                Layout::Flat
            },
            inventory,
            authorities: case_authorities,
            ownership_staged: Some(staged),
            ownership_authorities: case
                .authorities
                .iter()
                .map(|path| repository.join(path))
                .collect(),
        });
    }
    let plan = CasePlan { cases: planned };
    let report = plan.report();
    if mode == Mode::Apply {
        let digest = plan.transaction_digest();
        let retained = retained_transactions(repository, &digest)?;
        let complete = plan
            .cases
            .iter()
            .all(|case| case.layout == Layout::Directory);
        if complete {
            for root in retained.committed {
                garbage_collect(io, &root).with_context(|| {
                    format!(
                        "committed fixture cohort is complete; garbage collection failed; \
                         committed=true; retained owned transaction={}",
                        root.display()
                    )
                })?;
            }
        } else {
            ensure!(
                retained.committed.is_empty(),
                "owned committed transaction exists but installed cohort is incomplete"
            );
            AtomicCaseTransaction::new(repository, plan, digest, io)?.apply()?;
        }
    }
    Ok(report)
}

#[derive(Clone)]
struct CohortPathRole {
    path: PathBuf,
    role: &'static str,
    case: usize,
}

fn validate_cohort_path_ownership(cases: &[CohortCase]) -> Result<()> {
    ensure!(!cases.is_empty(), "cohort plan has no cases");
    let mut paths = Vec::new();
    for (case, entry) in cases.iter().enumerate() {
        for (role, value) in [
            ("staged case", entry.staged.as_str()),
            ("destination", entry.destination.as_str()),
        ] {
            validate_relative(value)?;
            paths.push(CohortPathRole {
                path: PathBuf::from(value),
                role,
                case,
            });
        }
        for authority in &entry.authorities {
            validate_relative(authority)?;
            paths.push(CohortPathRole {
                path: PathBuf::from(authority),
                role: "authority",
                case,
            });
        }
    }
    for path in &paths {
        let first = path.path.components().next();
        ensure!(
            !matches!(first, Some(Component::Normal(name)) if name.to_string_lossy().starts_with(TRANSACTION_PREFIX)),
            "{} {} overlaps the transaction-root namespace",
            path.role,
            path.path.display()
        );
    }
    for (index, left) in paths.iter().enumerate() {
        for right in &paths[index + 1..] {
            if left.case == right.case
                && left.path == right.path
                && matches!(
                    (left.role, right.role),
                    ("destination", "authority") | ("authority", "destination")
                )
            {
                continue;
            }
            if left.path.starts_with(&right.path) || right.path.starts_with(&left.path) {
                bail!(
                    "cohort path ownership collision: case {} {} {} overlaps case {} {} {}",
                    left.case,
                    left.role,
                    left.path.display(),
                    right.case,
                    right.role,
                    right.path.display()
                );
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    Flat,
    Directory,
}

/// One complete destination artifact, including its exact byte inventory and
/// every old authority consumed when it is installed.
struct ArtifactSpec {
    area: PathBuf,
    case: String,
    display_area: String,
    layout: Layout,
    inventory: BTreeMap<String, Vec<u8>>,
    authorities: Vec<PathBuf>,
    ownership_staged: Option<PathBuf>,
    ownership_authorities: Vec<PathBuf>,
}

/// The sole fixture publication plan. Every publication path is reduced to
/// this closed set before the filesystem is mutated.
struct CasePlan {
    cases: Vec<ArtifactSpec>,
}

impl CasePlan {
    fn report(&self) -> String {
        let mut report = String::new();
        for case in &self.cases {
            report.push_str(&format!(
                "{}/{}: {} files {} bytes sha256={}\n",
                case.display_area,
                case.case,
                case.inventory.len(),
                case.inventory.values().map(Vec::len).sum::<usize>(),
                inventory_digest(&case.inventory)
            ));
        }
        report
    }

    fn transaction_digest(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(TRANSACTION_SCHEMA.as_bytes());
        digest.update(b"\0canonical-plan-v2\0");
        let mut cases = self.cases.iter().collect::<Vec<_>>();
        cases.sort_by_key(|case| case.area.join(&case.case));
        for case in cases {
            digest_path(&mut digest, &case.area.join(&case.case));
            digest.update(inventory_digest(&case.inventory).as_bytes());
            if let Some(staged) = &case.ownership_staged {
                digest.update([1]);
                digest_path(&mut digest, staged);
                let mut authorities = case.ownership_authorities.iter().collect::<Vec<_>>();
                authorities.sort();
                digest.update((authorities.len() as u64).to_be_bytes());
                for authority in authorities {
                    digest_path(&mut digest, authority);
                }
            } else {
                digest.update([0]);
            }
        }
        format!("{:x}", digest.finalize())
    }
}

fn digest_path(digest: &mut Sha256, path: &Path) {
    let value = path.to_string_lossy();
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}

fn classic_case_metadata(case: &str, inventory: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut files = Vec::new();
    for (name, bytes) in inventory {
        let role = match Path::new(name).extension().and_then(|value| value.to_str()) {
            Some("aux" | "bib" | "bst") => "input",
            Some("bbl" | "blg" | "terminal") => "output",
            _ => bail!("classic BibTeX case {case} has unclassified payload {name}"),
        };
        files.push(serde_json::json!({
            "path": name,
            "role": role,
            "bytes": bytes.len(),
            "sha256": format!("{:x}", Sha256::digest(bytes)),
        }));
    }
    let value = serde_json::json!({
        "schema": "classic-bibtex-closed-case-v1",
        "case": case,
        "compatibility": "classic-bibtex-0.99d-texlive-2025-web2c",
        "files": files,
    });
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// The sole fixture publication transaction.
struct AtomicCaseTransaction<'a> {
    corpus: &'a Path,
    plan: CasePlan,
    root: PathBuf,
    digest: String,
    io: &'a dyn TransactionFs,
}

impl<'a> AtomicCaseTransaction<'a> {
    fn new(
        corpus: &'a Path,
        plan: CasePlan,
        digest: String,
        io: &'a dyn TransactionFs,
    ) -> Result<Self> {
        static NEXT_TRANSACTION: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT_TRANSACTION.fetch_add(1, Ordering::Relaxed);
        let mut root = None;
        for attempt in 0..1_000_u64 {
            let candidate = corpus.join(format!(
                "{TRANSACTION_PREFIX}{}-{sequence}-{attempt}",
                std::process::id()
            ));
            match io.create_dir(&candidate) {
                Ok(()) => {
                    root = Some(candidate);
                    break;
                }
                Err(error) if candidate.exists() => continue,
                Err(error) => return Err(error),
            }
        }
        let root = root.ok_or_else(|| {
            anyhow!(
                "could not allocate a unique fixture-layout transaction beside {}",
                corpus.display()
            )
        })?;
        if let Err(error) = io.write(&root.join(OWNER_MARKER), owner_marker(&digest).as_bytes()) {
            let _ = io.remove_dir_all(&root);
            return Err(error);
        }
        Ok(Self {
            corpus,
            plan,
            root,
            digest,
            io,
        })
    }

    fn apply(self) -> Result<()> {
        self.stage_all()?;
        let authorities = self.unique_authorities();
        let flat_cases: Vec<_> = self
            .plan
            .cases
            .iter()
            .filter(|case| case.layout == Layout::Flat)
            .collect();
        let mut backed_up = Vec::new();
        let mut installed = Vec::new();
        let commit: Result<()> = (|| {
            for source in authorities {
                let backup = self.backup_path(&source)?;
                self.io.rename(&source, &backup)?;
                backed_up.push((source, backup));
            }
            for case in flat_cases {
                let staged = self.staged_path(case)?;
                let target = case.area.join(&case.case);
                self.io.rename(&staged, &target)?;
                installed.push((target, staged));
            }
            self.validate_installed()?;
            self.io.write(
                &self.root.join(COMMITTED_MARKER),
                committed_marker(&self.digest).as_bytes(),
            )?;
            Ok(())
        })();
        if let Err(original) = commit {
            return Err(self.rollback_error(
                "fixture publication commit failed",
                original,
                &installed,
                &backed_up,
            ));
        }
        if let Err(cleanup) = garbage_collect(self.io, &self.root) {
            bail!(
                "fixture publication committed and revalidated; garbage collection failed: \
                 {cleanup:#}; committed=true; retained owned transaction={}",
                self.root.display()
            );
        }
        Ok(())
    }

    fn validate_installed(&self) -> Result<()> {
        for case in &self.plan.cases {
            let target = case.area.join(&case.case);
            if read_regular_inventory_recursive(&target)? != case.inventory {
                bail!(
                    "installed byte inventory changed for {}/{}",
                    case.display_area,
                    case.case
                );
            }
        }
        Ok(())
    }

    fn stage_all(&self) -> Result<()> {
        let result = (|| {
            for case in self
                .plan
                .cases
                .iter()
                .filter(|case| case.layout == Layout::Flat)
            {
                let staged = self.staged_path(case)?;
                self.io.create_dir_all(&staged)?;
                for (name, bytes) in &case.inventory {
                    let destination = staged.join(name);
                    if let Some(parent) = destination.parent() {
                        self.io.create_dir_all(parent)?;
                    }
                    self.io.write(&destination, bytes)?;
                }
                if read_regular_inventory_recursive(&staged)? != case.inventory {
                    bail!(
                        "staged byte inventory changed for {}/{}",
                        case.display_area,
                        case.case
                    );
                }
            }
            for source in self.unique_authorities() {
                let backup = self.backup_path(&source)?;
                if let Some(parent) = backup.parent() {
                    self.io.create_dir_all(parent)?;
                }
            }
            Ok(())
        })();
        if let Err(original) = result {
            return match self.io.remove_dir_all(&self.root) {
                Ok(()) => Err(original),
                Err(cleanup) => bail!(
                    "fixture publication staging failed: {original:#}; transaction cleanup failed: {cleanup:#}; recoverable transaction retained at {}",
                    self.root.display()
                ),
            };
        }
        Ok(())
    }

    fn rollback_error(
        &self,
        operation: &str,
        original: anyhow::Error,
        installed: &[(PathBuf, PathBuf)],
        backed_up: &[(PathBuf, PathBuf)],
    ) -> anyhow::Error {
        let mut failures = self.rollback(installed, backed_up);
        if failures.is_empty() {
            match self.io.remove_dir_all(&self.root) {
                Ok(()) => {
                    return original.context(format!("{operation}; every authority was restored"));
                }
                Err(error) => failures.push(format!(
                    "remove restored transaction root {}: {error:#}",
                    self.root.display()
                )),
            }
        }
        anyhow!(
            "{operation}: {original:#}; rollback failures: {}; recoverable transaction retained at {}",
            failures.join("; "),
            self.root.display()
        )
    }

    fn rollback(
        &self,
        installed: &[(PathBuf, PathBuf)],
        backed_up: &[(PathBuf, PathBuf)],
    ) -> Vec<String> {
        let mut failures = Vec::new();
        for (target, staged) in installed.iter().rev() {
            if let Err(error) = self.io.rename(target, staged) {
                failures.push(format!("restore installed {}: {error:#}", target.display()));
            }
        }
        for (source, backup) in backed_up.iter().rev() {
            if let Err(error) = self.io.rename(backup, source) {
                failures.push(format!(
                    "restore authority {} from {}: {error:#}",
                    source.display(),
                    backup.display()
                ));
            }
        }
        failures
    }

    fn unique_authorities(&self) -> Vec<PathBuf> {
        self.plan
            .cases
            .iter()
            .flat_map(|case| case.authorities.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn staged_path(&self, case: &ArtifactSpec) -> Result<PathBuf> {
        Ok(self
            .root
            .join("staged")
            .join(case.area.strip_prefix(self.corpus)?)
            .join(&case.case))
    }

    fn backup_path(&self, source: &Path) -> Result<PathBuf> {
        Ok(self
            .root
            .join("backup")
            .join(source.strip_prefix(self.corpus)?))
    }
}

fn garbage_collect(io: &dyn TransactionFs, root: &Path) -> Result<()> {
    // Keep the ownership and commit markers until all recursively removed
    // subtrees are gone. A partial recursive failure therefore remains
    // authenticated and resumable.
    io.remove_dir_all(&root.join("backup"))?;
    io.remove_dir_all(&root.join("staged"))?;
    io.remove_file(&root.join(COMMITTED_MARKER))?;
    io.remove_file(&root.join(OWNER_MARKER))?;
    io.remove_dir(root)
}

struct RetainedTransactions {
    committed: Vec<PathBuf>,
}

fn retained_transactions(corpus: &Path, digest: &str) -> Result<RetainedTransactions> {
    let expected_owner = owner_marker(digest);
    let expected_committed = committed_marker(digest);
    let mut committed = Vec::new();
    for entry in fs::read_dir(corpus).with_context(|| format!("read {}", corpus.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = file_name(&path)?;
        if !name.starts_with(TRANSACTION_PREFIX) {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            bail!("refusing non-directory transaction path {}", path.display());
        }
        let owner = fs::read_to_string(path.join(OWNER_MARKER)).with_context(|| {
            format!(
                "refusing unknown transaction root {}; ownership marker is absent or unreadable",
                path.display()
            )
        })?;
        if owner != expected_owner {
            bail!(
                "refusing mismatched transaction root {}; ownership marker does not match this plan",
                path.display()
            );
        }
        let committed_path = path.join(COMMITTED_MARKER);
        if committed_path.exists() {
            let marker = fs::read_to_string(&committed_path)
                .with_context(|| format!("read committed marker {}", committed_path.display()))?;
            if marker != expected_committed {
                bail!(
                    "refusing mismatched committed transaction root {}",
                    path.display()
                );
            }
            committed.push(path);
        }
    }
    committed.sort();
    Ok(RetainedTransactions { committed })
}

fn owner_marker(digest: &str) -> String {
    format!("{TRANSACTION_SCHEMA}\nplan-sha256={digest}\n")
}

fn committed_marker(digest: &str) -> String {
    format!("{TRANSACTION_SCHEMA}\nplan-sha256={digest}\nstate=committed\n")
}

pub(crate) trait TransactionFs {
    fn create_dir(&self, path: &Path) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove_dir_all(&self, path: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn remove_dir(&self, path: &Path) -> Result<()>;
}

pub(crate) struct RealFs;

impl TransactionFs for RealFs {
    fn create_dir(&self, path: &Path) -> Result<()> {
        fs::create_dir(path).with_context(|| format!("create unique {}", path.display()))
    }
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to)
            .with_context(|| format!("rename {} to {}", from.display(), to.display()))
    }
    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        if path.exists() {
            fs::remove_dir_all(path).with_context(|| format!("remove {}", path.display()))
        } else {
            Ok(())
        }
    }
    fn remove_file(&self, path: &Path) -> Result<()> {
        if path.exists() {
            fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
        } else {
            Ok(())
        }
    }
    fn remove_dir(&self, path: &Path) -> Result<()> {
        if path.exists() {
            fs::remove_dir(path).with_context(|| format!("remove {}", path.display()))
        } else {
            Ok(())
        }
    }
}

fn read_regular_inventory_recursive(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    fn visit(root: &Path, at: &Path, inventory: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(at)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                bail!("staged symlink {}", entry.path().display());
            } else if kind.is_dir() {
                visit(root, &entry.path(), inventory)?;
            } else if kind.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                inventory.insert(relative, fs::read(entry.path())?);
            } else {
                bail!("staged non-regular entry {}", entry.path().display());
            }
        }
        Ok(())
    }
    let mut inventory = BTreeMap::new();
    visit(directory, directory, &mut inventory)?;
    Ok(inventory)
}

fn inventory_digest(inventory: &BTreeMap<String, Vec<u8>>) -> String {
    let mut digest = Sha256::new();
    for (name, bytes) in inventory {
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    format!("{:x}", digest.finalize())
}

fn validate_relative(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe relative path {value:?}");
    }
    Ok(())
}

fn validate_component(value: &str) -> Result<()> {
    validate_relative(value)?;
    if Path::new(value).components().count() != 1 {
        bail!("unsafe path component {value:?}");
    }
    Ok(())
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("invalid UTF-8 file name {}", path.display()))
}
