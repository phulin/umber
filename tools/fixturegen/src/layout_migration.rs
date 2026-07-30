use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail, ensure};
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
pub struct SelectedSharedFile {
    pub cases: &'static [&'static str],
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
    /// Shared authorities copied only into the named cases, then consumed once.
    pub selected_shared_files: &'static [SelectedSharedFile],
}

#[allow(dead_code)] // Retain the completed execution cohort as an auditable reusable specification.
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
const NO_SELECTED_SHARED: &[SelectedSharedFile] = &[];
const TRANSACTION_PREFIX: &str = ".fixture-layout-transaction-";
const OWNER_MARKER: &str = "owner";
const COMMITTED_MARKER: &str = "committed";
const TRANSACTION_SCHEMA: &str = "umber-fixture-layout-transaction-v1";

#[allow(dead_code)] // Retain the completed execution cohort as an auditable reusable specification.
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

const SOURCE_TEX: CaseFile = CaseFile {
    source_suffix: ".tex",
    destination_suffix: "source.tex",
    destination_keeps_case: false,
    captures_tail: false,
    role: FileRole::Source,
    required: true,
};
const EXPECTED_TOKENS: CaseFile = CaseFile {
    source_suffix: ".expected.tokens",
    destination_suffix: "expected.tokens",
    destination_keeps_case: false,
    captures_tail: false,
    role: FileRole::Output,
    required: true,
};
const EXPECTED_LOG: CaseFile = CaseFile {
    source_suffix: ".expected.log",
    destination_suffix: "expected.log",
    destination_keeps_case: false,
    captures_tail: false,
    role: FileRole::Output,
    required: true,
};
const EXPECTED_DVI: CaseFile = CaseFile {
    source_suffix: ".expected.dvi",
    destination_suffix: "expected.dvi",
    destination_keeps_case: false,
    captures_tail: false,
    role: FileRole::Output,
    required: true,
};

pub const LEXICAL_SESSION_FAMILIES: &[FamilySpec] = &[
    FamilySpec {
        area: "canonical-dvi",
        case_discovery_suffix: ".tex",
        case_files: &[SOURCE_TEX, EXPECTED_DVI],
        case_owned_files: NO_OWNED,
        shared_files: NO_SHARED,
        selected_shared_files: NO_SELECTED_SHARED,
    },
    FamilySpec {
        area: "hello",
        case_discovery_suffix: ".tex",
        case_files: &[SOURCE_TEX, EXPECTED_LOG],
        case_owned_files: NO_OWNED,
        shared_files: NO_SHARED,
        selected_shared_files: NO_SELECTED_SHARED,
    },
    FamilySpec {
        area: "lexer",
        case_discovery_suffix: ".tex",
        case_files: &[SOURCE_TEX, EXPECTED_TOKENS],
        case_owned_files: NO_OWNED,
        shared_files: NO_SHARED,
        selected_shared_files: NO_SELECTED_SHARED,
    },
    FamilySpec {
        area: "lexer_dynamic",
        case_discovery_suffix: ".tex",
        case_files: &[SOURCE_TEX, EXPECTED_TOKENS],
        case_owned_files: NO_OWNED,
        shared_files: NO_SHARED,
        selected_shared_files: NO_SELECTED_SHARED,
    },
    FamilySpec {
        area: "stabilization",
        case_discovery_suffix: ".tex",
        case_files: &[SOURCE_TEX],
        case_owned_files: NO_OWNED,
        shared_files: NO_SHARED,
        selected_shared_files: NO_SELECTED_SHARED,
    },
];

const BIB_INVOCATION_CASE_FILES: &[CaseFile] = &[
    CaseFile {
        source_suffix: ".invocation",
        destination_suffix: "invocation.case",
        destination_keeps_case: false,
        captures_tail: false,
        role: FileRole::Metadata,
        required: true,
    },
    CaseFile {
        source_suffix: ".inventory",
        destination_suffix: "case.inventory",
        destination_keeps_case: false,
        captures_tail: false,
        role: FileRole::Metadata,
        required: true,
    },
];

const BIB_INVOCATION_OWNED: &[CaseOwnedFile] = &[
    CaseOwnedFile {
        case: "bcf-success",
        source: "basic.expected.bbl",
        destination: "expected.bbl",
        role: FileRole::Output,
    },
    CaseOwnedFile {
        case: "invalid-output-format",
        source: "invalid.expected.stderr",
        destination: "expected.stderr",
        role: FileRole::Output,
    },
    CaseOwnedFile {
        case: "tool-mode",
        source: "tool.expected.bib",
        destination: "expected.bib",
        role: FileRole::Output,
    },
];

const BIB_INVOCATION_SHARED: &[SelectedSharedFile] = &[
    SelectedSharedFile {
        cases: &["bcf-success", "invalid-output-format"],
        source: "basic.bcf",
        destination: "basic.bcf",
        role: FileRole::Input,
    },
    SelectedSharedFile {
        cases: &["bcf-success", "tool-mode"],
        source: "basic.bib",
        destination: "basic.bib",
        role: FileRole::Input,
    },
    SelectedSharedFile {
        cases: &["bcf-success", "tool-mode"],
        source: "basic.expected.stdout",
        destination: "expected.stdout",
        role: FileRole::Output,
    },
];

pub const ALL_FAMILIES: &[FamilySpec] = &[
    EXECUTION_FAMILIES[0],
    EXECUTION_FAMILIES[1],
    EXECUTION_FAMILIES[2],
    EXECUTION_FAMILIES[3],
    EXECUTION_FAMILIES[4],
    EXECUTION_FAMILIES[5],
    EXECUTION_FAMILIES[6],
    EXECUTION_FAMILIES[7],
    LEXICAL_SESSION_FAMILIES[0],
    LEXICAL_SESSION_FAMILIES[1],
    LEXICAL_SESSION_FAMILIES[2],
    LEXICAL_SESSION_FAMILIES[3],
    LEXICAL_SESSION_FAMILIES[4],
    FamilySpec {
        area: "bib/invocation",
        case_discovery_suffix: ".invocation",
        case_files: BIB_INVOCATION_CASE_FILES,
        case_owned_files: BIB_INVOCATION_OWNED,
        shared_files: NO_SHARED,
        selected_shared_files: BIB_INVOCATION_SHARED,
    },
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
        selected_shared_files: NO_SELECTED_SHARED,
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
    if mode == Mode::Apply {
        let digest = plan.transaction_digest();
        let retained = retained_transactions(corpus, &digest)?;
        let complete = plan
            .cases
            .iter()
            .all(|case| case.layout == Layout::Directory);
        if complete {
            for root in retained.committed {
                garbage_collect(io, &root).with_context(|| {
                    format!(
                        "committed fixture layout is complete; garbage collection failed; \
                         committed=true; retained owned transaction={}",
                        root.display()
                    )
                })?;
            }
        } else {
            if !retained.committed.is_empty() {
                bail!(
                    "owned committed transaction exists but installed fixture layout is incomplete: {}",
                    retained.committed[0].display()
                );
            }
            Transaction::new(corpus, plan, digest, io)?.apply()?;
        }
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
                        if !is_shared_authority(spec, &area, authority) {
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

    fn transaction_digest(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(TRANSACTION_SCHEMA.as_bytes());
        for case in &self.cases {
            digest.update((case.display_area.len() as u64).to_be_bytes());
            digest.update(case.display_area.as_bytes());
            digest.update((case.case.len() as u64).to_be_bytes());
            digest.update(case.case.as_bytes());
            digest.update(inventory_digest(&case.inventory).as_bytes());
        }
        format!("{:x}", digest.finalize())
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
        if name.starts_with(TRANSACTION_PREFIX) {
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
    for mapping in spec
        .selected_shared_files
        .iter()
        .filter(|mapping| mapping.cases.contains(&case))
    {
        add_mapping(
            &mut inventory,
            &mut authorities,
            mapping.destination.to_owned(),
            area.join(mapping.source),
        )?;
    }
    Ok((inventory, authorities))
}

fn is_shared_authority(spec: &FamilySpec, area: &Path, authority: &Path) -> bool {
    spec.shared_files
        .iter()
        .any(|file| area.join(file.source) == authority)
        || spec
            .selected_shared_files
            .iter()
            .any(|file| area.join(file.source) == authority)
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
    validate_relative(spec.area)?;
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
    let mut selected = BTreeSet::new();
    for mapping in spec.selected_shared_files {
        validate_relative(mapping.source)?;
        validate_relative(mapping.destination)?;
        ensure!(
            !mapping.cases.is_empty(),
            "{} selected shared file {} has no cases",
            spec.area,
            mapping.source
        );
        selected.clear();
        for case in mapping.cases {
            validate_component(case)?;
            ensure!(
                selected.insert(*case),
                "{} selected shared file {} repeats case {}",
                spec.area,
                mapping.source,
                case
            );
        }
    }
    Ok(())
}

struct Transaction<'a> {
    corpus: &'a Path,
    plan: MigrationPlan,
    root: PathBuf,
    digest: String,
    io: &'a dyn MigrationFs,
}

impl<'a> Transaction<'a> {
    fn new(
        corpus: &'a Path,
        plan: MigrationPlan,
        digest: String,
        io: &'a dyn MigrationFs,
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
            self.validate_installed(&installed)?;
            self.io.write(
                &self.root.join(COMMITTED_MARKER),
                committed_marker(&self.digest).as_bytes(),
            )?;
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
        if let Err(cleanup) = garbage_collect(self.io, &self.root) {
            bail!(
                "fixture layout committed and revalidated; garbage collection failed: \
                 {cleanup:#}; committed=true; retained owned transaction={}",
                self.root.display()
            );
        }
        Ok(())
    }

    fn validate_installed(&self, installed: &[(PathBuf, PathBuf)]) -> Result<()> {
        for (target, _) in installed {
            let case = self
                .plan
                .cases
                .iter()
                .find(|case| case.area.join(&case.case) == *target)
                .ok_or_else(|| {
                    anyhow!("installed target is absent from plan: {}", target.display())
                })?;
            if read_regular_inventory_recursive(target)? != case.inventory {
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

fn garbage_collect(io: &dyn MigrationFs, root: &Path) -> Result<()> {
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

trait MigrationFs {
    fn create_dir(&self, path: &Path) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove_dir_all(&self, path: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn remove_dir(&self, path: &Path) -> Result<()>;
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
