use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Metadata is part of the reusable schema; this cohort has none.
pub enum FileRole {
    Source,
    Input,
    Metadata,
    Output,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaseFile {
    pub source_suffix: &'static str,
    pub destination_suffix: &'static str,
    pub destination_keeps_case: bool,
    pub captures_tail: bool,
    pub role: FileRole,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaseOwnedFile {
    pub case: &'static str,
    pub source: &'static str,
    pub destination: &'static str,
    pub role: FileRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedFile {
    pub source: &'static str,
    pub destination: &'static str,
    pub role: FileRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FamilySpec {
    pub area: &'static str,
    /// The suffix of the authority file that discovers and names a flat case.
    pub case_discovery_suffix: &'static str,
    /// Per-case mappings. `source_suffix` and `destination_suffix` are appended
    /// to the discovered case name; an empty destination is allowed.
    pub case_files: &'static [CaseFile],
    pub case_owned_files: &'static [CaseOwnedFile],
    /// Shared authorities are copied into every case, then consumed once.
    pub shared_files: &'static [SharedFile],
}

const EXECUTION_CASE_FILES: &[CaseFile] = &[
    CaseFile {
        source_suffix: ".tex",
        destination_suffix: ".tex",
        destination_keeps_case: true,
        captures_tail: false,
        role: FileRole::Source,
        required: true,
    },
    CaseFile {
        source_suffix: ".expected.",
        destination_suffix: "expected.",
        destination_keeps_case: false,
        captures_tail: true,
        role: FileRole::Output,
        required: false,
    },
];
const NO_OWNED: &[CaseOwnedFile] = &[];
const NO_SHARED: &[SharedFile] = &[];

pub const EXECUTION_FAMILIES: &[FamilySpec] = &[
    execution_family("align", NO_OWNED, NO_SHARED),
    execution_family(
        "etex_exec",
        &[CaseOwnedFile {
            case: "expansion_virtual_input",
            source: "expansion_virtual_input.txt",
            destination: "expansion_virtual_input.txt",
            role: FileRole::Input,
        }],
        NO_SHARED,
    ),
    execution_family("exec", NO_OWNED, NO_SHARED),
    execution_family(
        "expand",
        &[CaseOwnedFile {
            case: "input_main",
            source: "input_secondary.inc",
            destination: "input_secondary.inc",
            role: FileRole::Input,
        }],
        NO_SHARED,
    ),
    execution_family(
        "math",
        NO_OWNED,
        &[SharedFile {
            source: "math_preamble.inc",
            destination: "math_preamble.inc",
            role: FileRole::Input,
        }],
    ),
    execution_family("tex_exec", NO_OWNED, NO_SHARED),
    execution_family("tex_exec_io", NO_OWNED, NO_SHARED),
    execution_family("typeset", NO_OWNED, NO_SHARED),
];

const fn execution_family(
    area: &'static str,
    case_owned_files: &'static [CaseOwnedFile],
    shared_files: &'static [SharedFile],
) -> FamilySpec {
    FamilySpec {
        area,
        case_discovery_suffix: ".tex",
        case_files: EXECUTION_CASE_FILES,
        case_owned_files,
        shared_files,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Plan,
    Apply,
}

pub fn run(corpus: &Path, specs: &[FamilySpec], mode: Mode) -> Result<String> {
    run_with_fs(corpus, specs, mode, &RealFs)
}

fn run_with_fs(
    corpus: &Path,
    specs: &[FamilySpec],
    mode: Mode,
    io: &dyn MigrationFs,
) -> Result<String> {
    let plan = MigrationPlan::build(corpus, specs)?;
    let report = plan.report();
    if mode == Mode::Apply
        && !plan
            .cases
            .iter()
            .all(|case| case.layout == Layout::Directory)
    {
        Transaction::new(corpus, plan, io)?.apply()?;
    }
    Ok(report)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    Flat,
    Directory,
}

struct PlannedCase {
    area: PathBuf,
    case: String,
    display_area: &'static str,
    layout: Layout,
    inventory: BTreeMap<String, Vec<u8>>,
    authorities: Vec<PathBuf>,
}

struct MigrationPlan {
    cases: Vec<PlannedCase>,
}

impl MigrationPlan {
    fn build(corpus: &Path, specs: &[FamilySpec]) -> Result<Self> {
        let mut planned = Vec::new();
        let mut authority_owner = BTreeMap::<PathBuf, String>::new();
        for spec in specs {
            validate_spec(spec)?;
            let area = corpus.join(spec.area);
            let cases = discover_cases(&area, spec.case_discovery_suffix)?;
            if cases.is_empty() {
                bail!("{} has no flat or directory cases", area.display());
            }
            for (case, layout) in cases {
                validate_component(&case)?;
                let (inventory, authorities) = inventory_for_case(&area, spec, &case, layout)?;
                for authority in &authorities {
                    if let Some(owner) =
                        authority_owner.insert(authority.clone(), format!("{}/{case}", spec.area))
                    {
                        if !spec
                            .shared_files
                            .iter()
                            .any(|file| area.join(file.source) == *authority)
                        {
                            bail!(
                                "authority {} is owned by both {owner} and {}/{case}",
                                authority.display(),
                                spec.area
                            );
                        }
                    }
                }
                planned.push(PlannedCase {
                    area: area.clone(),
                    case,
                    display_area: spec.area,
                    layout,
                    inventory,
                    authorities,
                });
            }
        }
        Ok(Self { cases: planned })
    }

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
}

fn discover_cases(area: &Path, discovery_suffix: &str) -> Result<BTreeMap<String, Layout>> {
    let mut cases = BTreeMap::new();
    for entry in fs::read_dir(area).with_context(|| format!("read {}", area.display()))? {
        let entry = entry.context("read area entry")?;
        let path = entry.path();
        let kind = entry.file_type().context("read area entry type")?;
        if kind.is_symlink() {
            bail!("symlink is forbidden: {}", path.display());
        }
        let name = file_name(&path)?;
        if name.starts_with(".fixture-layout-transaction-") {
            continue;
        }
        let discovered = if kind.is_dir() {
            Some((name, Layout::Directory))
        } else if kind.is_file() {
            name.strip_suffix(discovery_suffix)
                .filter(|case| !case.is_empty())
                .map(|case| (case.to_owned(), Layout::Flat))
        } else {
            None
        };
        if let Some((case, layout)) = discovered {
            if cases.insert(case.clone(), layout).is_some() {
                bail!("flat/directory collision for case {case}");
            }
        }
    }
    Ok(cases)
}

fn inventory_for_case(
    area: &Path,
    spec: &FamilySpec,
    case: &str,
    layout: Layout,
) -> Result<(BTreeMap<String, Vec<u8>>, Vec<PathBuf>)> {
    if layout == Layout::Directory {
        return Ok((
            read_regular_inventory_recursive(&area.join(case))?,
            Vec::new(),
        ));
    }
    let mut inventory = BTreeMap::new();
    let mut authorities = Vec::new();
    for mapping in spec.case_files {
        let source_prefix = format!("{case}{}", mapping.source_suffix);
        let matches = matching_files(
            area,
            &source_prefix,
            mapping.required,
            mapping.captures_tail,
        )?;
        for (source_name, tail) in matches {
            let destination = format!(
                "{}{}{}",
                if mapping.destination_keeps_case {
                    case
                } else {
                    ""
                },
                mapping.destination_suffix,
                tail
            );
            add_mapping(
                &mut inventory,
                &mut authorities,
                destination,
                area.join(source_name),
            )?;
        }
    }
    for mapping in spec
        .case_owned_files
        .iter()
        .filter(|mapping| mapping.case == case)
    {
        add_mapping(
            &mut inventory,
            &mut authorities,
            mapping.destination.to_owned(),
            area.join(mapping.source),
        )?;
    }
    for mapping in spec.shared_files {
        add_mapping(
            &mut inventory,
            &mut authorities,
            mapping.destination.to_owned(),
            area.join(mapping.source),
        )?;
    }
    Ok((inventory, authorities))
}

fn matching_files(
    area: &Path,
    prefix: &str,
    required: bool,
    captures_tail: bool,
) -> Result<Vec<(String, String)>> {
    let mut matches = Vec::new();
    for entry in fs::read_dir(area)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = file_name(&entry.path())?;
        if let Some(tail) = name
            .strip_prefix(prefix)
            .filter(|tail| captures_tail || tail.is_empty())
        {
            let tail = tail.to_owned();
            matches.push((name, tail));
        }
    }
    if required && matches.is_empty() {
        bail!(
            "required authority matching {prefix:?} is absent in {}",
            area.display()
        );
    }
    Ok(matches)
}

fn add_mapping(
    inventory: &mut BTreeMap<String, Vec<u8>>,
    authorities: &mut Vec<PathBuf>,
    destination: String,
    source: PathBuf,
) -> Result<()> {
    validate_relative(&destination)?;
    let bytes = fs::read(&source).with_context(|| format!("read {}", source.display()))?;
    if inventory.insert(destination.clone(), bytes).is_some() {
        bail!("duplicate destination {destination}");
    }
    authorities.push(source);
    Ok(())
}

fn validate_spec(spec: &FamilySpec) -> Result<()> {
    validate_component(spec.area)?;
    if spec.case_discovery_suffix.is_empty() {
        bail!("{} has an empty case discovery suffix", spec.area);
    }
    for mapping in spec.case_files {
        if mapping.source_suffix.is_empty() {
            bail!("{} has an empty case-file source suffix", spec.area);
        }
        validate_template_suffix(mapping.destination_suffix)?;
    }
    for mapping in spec.case_owned_files {
        validate_component(mapping.case)?;
        validate_relative(mapping.source)?;
        validate_relative(mapping.destination)?;
    }
    for mapping in spec.shared_files {
        validate_relative(mapping.source)?;
        validate_relative(mapping.destination)?;
    }
    Ok(())
}

struct Transaction<'a> {
    corpus: &'a Path,
    plan: MigrationPlan,
    root: PathBuf,
    io: &'a dyn MigrationFs,
}

impl<'a> Transaction<'a> {
    fn new(corpus: &'a Path, plan: MigrationPlan, io: &'a dyn MigrationFs) -> Result<Self> {
        static NEXT_TRANSACTION: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT_TRANSACTION.fetch_add(1, Ordering::Relaxed);
        let mut root = None;
        for attempt in 0..1_000_u64 {
            let candidate = corpus.join(format!(
                ".fixture-layout-transaction-{}-{sequence}-{attempt}",
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
        Ok(Self {
            corpus,
            plan,
            root,
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
            Ok(())
        })();
        if let Err(original) = commit {
            return Err(self.rollback_error(
                "migration commit failed",
                original,
                &installed,
                &backed_up,
            ));
        }
        if let Err(cleanup) = self.io.remove_dir_all(&self.root) {
            return Err(self.rollback_error(
                "migration cleanup failed after swaps committed",
                cleanup,
                &installed,
                &backed_up,
            ));
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
                    "migration staging failed: {original:#}; transaction cleanup failed: {cleanup:#}; recoverable transaction retained at {}",
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

    fn staged_path(&self, case: &PlannedCase) -> Result<PathBuf> {
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

trait MigrationFs {
    fn create_dir(&self, path: &Path) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove_dir_all(&self, path: &Path) -> Result<()>;
}

struct RealFs;

impl MigrationFs for RealFs {
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

fn validate_template_suffix(value: &str) -> Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    validate_relative(value.trim_start_matches('/'))
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

#[cfg(test)]
mod tests;
